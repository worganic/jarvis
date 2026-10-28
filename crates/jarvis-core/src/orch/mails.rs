// Mails numérotés et card de lecture (orchestrator/mails.js) : « ouvre le mail 3 », « lis le mail 2 »,
// « résume le mail 1 », « réponds au mail 4 », « n'annonce plus les mails de l'expéditeur du mail 3 ».
// Le mail est identifié par son UID IMAP et lu sans être marqué comme lu.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

use chrono::Timelike;
use serde_json::{json, Value};

use super::intent::Intent;
use super::mail_filters::{address_of, name_of};
use super::mail_parse::clean_text;
use super::mail_watch::parse_mail_date;
use super::projects::spoken_number;
use super::{OrchRef, Up};
use crate::js::{self, J};

const CACHE_MS: i64 = 10 * 60 * 1000;
const NUM: &str = r"(?:numero |n |no )?(\d{1,3}|un|une|premier|premiere|deux|trois|quatre|cinq|six|sept|huit|neuf|dix|onze|douze)";
const MAIL: &str = r"(?:le |l |ce )?(?:mail|email|e mail|message|courriel)";

static OPEN: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(&format!(r"^(?:ouvre|ouvrir|affiche|montre|consulte|detaille|lis|lire|relis)(?: moi)? {MAIL} {NUM}(?: dans (gmail|le navigateur|chrome))?$")).unwrap());
static SUMMARY: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(&format!(r"^(?:resume|resumer)(?: moi)? {MAIL} {NUM}$")).unwrap());
static SUMMARY2: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(&format!(r"^(?:fais|donne)(?: moi)? (?:un |le )?resume (?:du |de l )(?:mail|email|e mail|message|courriel) {NUM}$")).unwrap());
static REPLY: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(&format!(r"^(?:reponds|repondre|prepare une reponse|redige une reponse|ecris une reponse)(?: moi)? (?:au|a l|a ce) (?:mail|email|e mail|message|courriel) {NUM}$")).unwrap());
static EXCLUDE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(&format!(r"^(?:(?:n annonce plus|ne m annonce plus|ignore|exclus|exclue|bloque|masque) (?:les )?(?:mails|messages|emails) (?:de l expediteur )?(?:du|de l) (?:mail|email|message) {NUM}|(?:exclus|exclue|bloque|ignore) l expediteur (?:du|de l) (?:mail|email|message) {NUM})$")).unwrap()
});

fn long_date(raw: &str) -> String {
    match parse_mail_date(raw) {
        Some(d) => {
            use chrono::Datelike;
            format!("{} {} {} {} à {:02}:{:02}", js::JOURS[d.weekday().num_days_from_sunday() as usize], d.day(), js::MOIS[d.month0() as usize], d.year(), d.hour(), d.minute())
        }
        None => raw.to_string(),
    }
}

/// Texte lu à voix haute : sans URL, sans lignes de signature, raccourci.
pub fn speakable_text(text: &str, max: usize) -> String {
    let t = crate::re!(r"https?://\S+").replace_all(text, " ");
    let t = crate::re!(r"\([^)]{0,3}\)").replace_all(&t, " ");
    let joined = t
        .split('\n')
        .filter(|l| !l.trim().is_empty() && !crate::re!(r"(?i)^(?:>|--\s*$|envoy[ée] (?:de|depuis) mon|se d[ée]sinscrire|unsubscribe)").is_match(l.trim()))
        .collect::<Vec<_>>()
        .join(" ");
    let clean = crate::re!(r"\s+").replace_all(&joined, " ").trim().to_string();
    if clean.chars().count() <= max {
        return clean;
    }
    let cut: String = clean.chars().take(max).collect();
    let end = [cut.rfind(". "), cut.rfind("! "), cut.rfind("? ")].into_iter().flatten().max();
    match end {
        Some(e) if cut[..e].chars().count() as f64 > max as f64 * 0.5 => cut[..e + 1].to_string(),
        _ => format!("{cut}…"),
    }
}

pub struct MailReader {
    o: OrchRef,
    #[allow(dead_code)]
    log: Rc<dyn Fn(String)>,
    numbered: RefCell<Vec<J>>,
    cache: RefCell<Vec<(i64, i64, J)>>, // (uid, instant, mail)
}

enum Target {
    Ok(i64, Option<i64>),
    Err(String),
}

impl MailReader {
    pub fn new(o: OrchRef, log: Rc<dyn Fn(String)>) -> Self {
        Self { o, log, numbered: RefCell::new(vec![]), cache: RefCell::new(vec![]) }
    }

    /// Une card de mails vient d'être affichée : ses numéros font foi.
    pub fn remember(&self, mails: &[J]) {
        *self.numbered.borrow_mut() = mails.iter().map(|m| json!({ "uid": m["uid"], "from": m["from"], "subject": m["subject"], "date": m["date"] })).collect();
    }

    fn by_number(&self, n: i64) -> Option<J> {
        if n < 1 {
            return None;
        }
        self.numbered.borrow().get((n - 1) as usize).cloned()
    }

    fn number_of(&self, uid: i64) -> Option<i64> {
        self.numbered.borrow().iter().position(|m| m["uid"].as_i64() == Some(uid)).map(|i| i as i64 + 1)
    }

    async fn fetch(&self, uid: i64) -> J {
        if let Some((_, _, m)) = self.cache.borrow().iter().find(|(u, at, _)| *u == uid && js::now_ms() - at < CACHE_MS) {
            return m.clone();
        }
        let o = self.o.up();
        let o2 = self.o.clone();
        let r = o
            .local_tool("mcp__google__mail_lire", json!({ "uid": uid }), Box::new(move |p| Box::pin(async move { o2.up().google.mail_lire(&p).await })))
            .await;
        if js::gb(&r, "ok") {
            let mut c = self.cache.borrow_mut();
            c.retain(|(u, _, _)| *u != uid);
            c.push((uid, js::now_ms(), r.clone()));
            if c.len() > 30 {
                c.remove(0);
            }
        }
        r
    }

    fn missing(n: i64) -> Target {
        Target::Err(format!("Il n'y a pas de mail {n} dans la dernière liste. Dis « lis mes mails » pour la réafficher."))
    }

    fn unknown_uid() -> Target {
        Target::Err("Ce mail ne peut pas être ouvert (identifiant inconnu) : réaffiche la liste avec « lis mes mails ».".into())
    }

    /// Désigné par un numéro de la dernière card, ou par son UID (clic sur la card).
    fn resolve(&self, r: &J) -> Target {
        match r {
            Value::Null => Target::Err("Quel mail ?".into()),
            Value::Object(_) => {
                if !r["uid"].is_null() {
                    let uid = js::num(&r["uid"]).unwrap_or(0.0) as i64;
                    let n = self.number_of(uid).or_else(|| r["n"].as_i64());
                    return Target::Ok(uid, n);
                }
                if !r["n"].is_null() {
                    let n = js::num(&r["n"]).unwrap_or(0.0) as i64;
                    return match self.by_number(n) {
                        None => Self::missing(n),
                        Some(m) if m["uid"].is_null() => Self::unknown_uid(),
                        Some(m) => Target::Ok(m["uid"].as_i64().unwrap_or(0), Some(n)),
                    };
                }
                Target::Err("Quel mail ?".into())
            }
            Value::Number(num) => {
                let v = num.as_f64().unwrap_or(0.0) as i64;
                if self.numbered.borrow().iter().any(|m| m["uid"].as_i64() == Some(v)) {
                    return Target::Ok(v, self.number_of(v));
                }
                if let Some(m) = self.by_number(v) {
                    return if m["uid"].is_null() { Self::unknown_uid() } else { Target::Ok(m["uid"].as_i64().unwrap_or(0), Some(v)) };
                }
                if !self.numbered.borrow().is_empty() && v > 0 && v <= 100 {
                    return Self::missing(v);
                }
                Target::Ok(v, self.number_of(v))
            }
            Value::String(s) => {
                if let Ok(p) = s.trim().parse::<f64>() {
                    if p > 0.0 {
                        return self.resolve(&json!(p as i64));
                    }
                }
                if let Some(n) = spoken_number(s) {
                    return match self.by_number(n) {
                        None => Self::missing(n),
                        Some(m) if m["uid"].is_null() => Self::unknown_uid(),
                        Some(m) => Target::Ok(m["uid"].as_i64().unwrap_or(0), Some(n)),
                    };
                }
                Target::Err("Quel mail ?".into())
            }
            _ => Target::Err("Quel mail ?".into()),
        }
    }

    fn card_data(mail: &J, n: Option<i64>) -> J {
        let from = js::gs(mail, "from");
        let name = name_of(&from);
        let address = address_of(&from);
        let subject = js::gs(mail, "subject");
        let fallback = if subject.is_empty() { "https://mail.google.com".to_string() } else { format!("https://mail.google.com/mail/u/0/#search/{}", urlencoding::encode(&subject)) };
        let text = js::slice_chars(&clean_text(&js::gs(mail, "text")), 30000);
        json!({
            "title": if subject.is_empty() { "(sans objet)".to_string() } else { subject.clone() },
            "uid": mail["uid"], "mailNo": n,
            "fromName": name, "fromAddress": if address != name { address } else { String::new() },
            "to": js::gs(mail, "to"), "cc": js::gs(mail, "cc"), "dateLabel": long_date(&js::gs(mail, "date")),
            "text": if text.is_empty() { "(message sans texte)".to_string() } else { text },
            "cut": js::gs(mail, "text").chars().count() > 30000 || js::gb(mail, "tronque"),
            "attachments": js::arr(&mail["attachments"]).into_iter().take(12).collect::<Vec<_>>(),
            "link": if js::truthy(&mail["lien"]) { js::gs(mail, "lien") } else { fallback }
        })
    }

    /// Ouvre la card de lecture ; speak : lit aussi le mail à voix haute.
    pub async fn open(&self, r: &J, speak: bool) -> J {
        let (uid, n) = match self.resolve(r) {
            Target::Err(e) => return json!({ "ok": false, "say": e }),
            Target::Ok(u, n) => (u, n),
        };
        let mail = self.fetch(uid).await;
        if !js::truthy(&mail) || mail["ok"] == json!(false) {
            let say = if js::truthy(&mail["message"]) { js::gs(&mail, "message") } else if js::truthy(&mail["error"]) { js::gs(&mail, "error") } else { "Je n’ai pas pu lire ce mail.".into() };
            return json!({ "ok": false, "say": say });
        }
        let o = self.o.up();
        o.show_widget("mail_view", Self::card_data(&mail, n));
        o.record("mail_ouvert", json!({ "expediteur": name_of(&js::gs(&mail, "from")), "objet": js::slice_chars(&js::gs(&mail, "subject"), 120) }));
        let who = { let w = name_of(&js::gs(&mail, "from")); if w.is_empty() { "un expéditeur inconnu".to_string() } else { w } };
        if !speak {
            return json!({ "ok": true, "say": null, "display": mail["subject"] });
        }
        json!({ "ok": true, "say": format!("Mail de {who}, objet : {}. {}", js::gs(&mail, "subject"), speakable_text(&js::gs(&mail, "text"), 700)), "display": mail["subject"] })
    }

    fn mail_prompt(mail: &J) -> String {
        let atts: Vec<String> = js::arr(&mail["attachments"]).iter().map(|a| js::gs(a, "name")).collect();
        [
            format!("De : {}", js::gs(mail, "from")),
            format!("À : {}", js::gs(mail, "to")),
            if js::truthy(&mail["cc"]) { format!("Cc : {}", js::gs(mail, "cc")) } else { String::new() },
            format!("Date : {}", js::gs(mail, "date")),
            format!("Objet : {}", js::gs(mail, "subject")),
            if atts.is_empty() { String::new() } else { format!("Pièces jointes : {}", atts.join(", ")) },
            String::new(),
            js::slice_chars(&js::gs(mail, "text"), 8000),
        ]
        .iter()
        .filter(|l| !l.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
    }

    /// Boutons de la card de lecture, et actions dites avec un numéro.
    pub async fn act(&self, r: &J, action: &str) -> J {
        let (uid, _) = match self.resolve(r) {
            Target::Err(e) => return json!({ "ok": false, "say": e }),
            Target::Ok(u, n) => (u, n),
        };
        if action == "ouvrir" {
            return self.open(&json!({ "uid": uid }), false).await;
        }
        if action == "lire" {
            return self.open(&json!({ "uid": uid }), true).await;
        }
        let mail = self.fetch(uid).await;
        if !js::truthy(&mail) || mail["ok"] == json!(false) {
            let say = if js::truthy(&mail["message"]) { js::gs(&mail, "message") } else if js::truthy(&mail["error"]) { js::gs(&mail, "error") } else { "Je n’ai pas pu lire ce mail.".into() };
            return json!({ "ok": false, "say": say });
        }
        let o = self.o.up();
        match action {
            "resumer" => {
                o.enqueue_ai(format!("Résume ce mail en deux ou trois phrases, en français, pour me le dire à voix haute (qui écrit, ce qu'il veut, ce que je dois faire éventuellement) :\n\n{}", Self::mail_prompt(&mail)), json!({ "priority": 1 }));
                json!({ "ok": true, "say": null, "display": "Résumé demandé" })
            }
            "repondre" => {
                o.enqueue_ai(format!("Propose-moi une réponse courte et adaptée à ce mail, en français. Lis-la moi et ne l'envoie pas tant que je ne te l'ai pas confirmé (l'envoi passe par mail_envoyer, avec mon code) :\n\n{}", Self::mail_prompt(&mail)), json!({ "priority": 1 }));
                json!({ "ok": true, "say": null, "display": "Réponse en préparation" })
            }
            "exclure" => {
                let r = o.mail_filters.add(&mail["from"]);
                if !js::gb(&r, "ok") {
                    return json!({ "ok": false, "say": "Je n'ai pas trouvé l'expéditeur de ce mail." });
                }
                let label = js::gs(&r["rule"], "label");
                json!({ "ok": true, "say": if js::gb(&r, "existed") { format!("Je n'annonçais déjà plus les mails de {label}.") } else { format!("C'est noté : je ne t'annoncerai plus les mails de {label}. Je l'ai ajouté à ma mémoire.") } })
            }
            "gmail" | "chrome" => {
                let subject = js::gs(&mail, "subject");
                let url = if js::truthy(&mail["lien"]) {
                    js::gs(&mail, "lien")
                } else if !subject.is_empty() {
                    format!("https://mail.google.com/mail/u/0/#search/{}", urlencoding::encode(&subject))
                } else {
                    "https://mail.google.com".into()
                };
                let r = o
                    .local_tool("mcp__system__ouvrir_url", json!({ "url": url }), Box::new(|p| Box::pin(async move { super::system::ouvrir_url(&p).await.unwrap_or_else(|e| json!({ "ok": false, "error": e })) })))
                    .await;
                if r["ok"] == json!(false) {
                    json!({ "ok": false, "say": if js::truthy(&r["error"]) { js::gs(&r, "error") } else { "Ouverture refusée.".into() } })
                } else {
                    json!({ "ok": true, "say": null })
                }
            }
            _ => json!({ "ok": false, "say": format!("Action inconnue : {action}") }),
        }
    }

    /// Intentions du routeur (texte replié).
    pub fn match_(&self, t: &str) -> Option<Intent> {
        let done = |r: J, display: String| {
            let d = if js::truthy(&r["display"]) { r["display"].clone() } else if !display.is_empty() { json!(display) } else if js::gb(&r, "ok") { json!("OK") } else { r["say"].clone() };
            json!({ "say": r["say"], "display": d })
        };
        if let Some(c) = OPEN.captures(t) {
            let n = spoken_number(&c[1]).unwrap_or(0);
            let o = self.o.clone();
            if c.get(2).is_some() {
                return Some(Intent::new("mail_gmail", "Mail", &format!("n° {n} dans Gmail"), move || {
                    let o = o.clone();
                    async move { done(o.up().mails.act(&json!(n), "gmail").await, "Gmail".into()) }
                }));
            }
            let speak = crate::re!(r"^(?:lis|lire|relis)\b").is_match(t);
            return Some(Intent::new("mail_open", "Mail", &format!("n° {n}"), move || {
                let o = o.clone();
                async move { done(o.up().mails.open(&json!(n), speak).await, format!("Mail {n}")) }
            }));
        }
        if let Some(c) = SUMMARY.captures(t).or_else(|| SUMMARY2.captures(t)) {
            let n = spoken_number(&c[1]).unwrap_or(0);
            let o = self.o.clone();
            return Some(Intent::new("mail_summary", "Mail", &format!("résumé du n° {n}"), move || {
                let o = o.clone();
                async move { done(o.up().mails.act(&json!(n), "resumer").await, format!("Résumé du mail {n}")) }
            }));
        }
        if let Some(c) = REPLY.captures(t) {
            let n = spoken_number(&c[1]).unwrap_or(0);
            let o = self.o.clone();
            return Some(Intent::new("mail_reply", "Mail", &format!("réponse au n° {n}"), move || {
                let o = o.clone();
                async move { done(o.up().mails.act(&json!(n), "repondre").await, format!("Réponse au mail {n}")) }
            }));
        }
        if let Some(c) = EXCLUDE.captures(t) {
            let raw = c.get(1).or_else(|| c.get(2)).map(|m| m.as_str()).unwrap_or("");
            let n = spoken_number(raw).unwrap_or(0);
            let o = self.o.clone();
            return Some(Intent::new("mail_exclude_sender", "Exclusion de mails", &format!("expéditeur du n° {n}"), move || {
                let o = o.clone();
                async move { done(o.up().mails.act(&json!(n), "exclure").await, "Expéditeur exclu".into()) }
            }));
        }
        None
    }
}
