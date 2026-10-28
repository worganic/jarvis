// Compétences locales sans IA (orchestrator/local-skills.js) : dates et heures en français, calcul,
// conversions d'unités, sites web connus, météo (Open-Meteo, sans clé). Les textes reçus sont déjà
// « repliés » (minuscules, sans accents).

use std::sync::LazyLock;

use chrono::{Datelike, Duration, Local, NaiveDate};
use serde_json::{json, Value};

use crate::js::{self, J};

pub const DAYS: [&str; 7] = ["dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi"];
pub const MONTHS: [&str; 12] = ["janvier", "fevrier", "mars", "avril", "mai", "juin", "juillet", "aout", "septembre", "octobre", "novembre", "decembre"];

pub fn word_number(w: &str) -> Option<f64> {
    Some(match w {
        "un" | "une" | "premier" => 1.0,
        "deux" => 2.0,
        "trois" => 3.0,
        "quatre" => 4.0,
        "cinq" => 5.0,
        "six" => 6.0,
        "sept" => 7.0,
        "huit" => 8.0,
        "neuf" => 9.0,
        "dix" => 10.0,
        "onze" => 11.0,
        "douze" => 12.0,
        "treize" => 13.0,
        "quatorze" => 14.0,
        "quinze" => 15.0,
        "seize" => 16.0,
        "vingt" => 20.0,
        "trente" => 30.0,
        "quarante" => 40.0,
        "cinquante" => 50.0,
        "soixante" => 60.0,
        "cent" => 100.0,
        "mille" => 1000.0,
        _ => return None,
    })
}

#[derive(Clone, Debug)]
pub struct Day {
    pub date: NaiveDate,
    pub iso: String,
    pub label: String,
}

pub fn format_day(d: NaiveDate) -> String {
    format!("{} {} {}", js::JOURS[d.weekday().num_days_from_sunday() as usize], d.day(), js::MOIS[d.month0() as usize])
}

fn day(d: NaiveDate, label: String) -> Day {
    Day { date: d, iso: d.format("%Y-%m-%d").to_string(), label }
}

fn add_months(d: NaiveDate, n: u32) -> NaiveDate {
    d.checked_add_months(chrono::Months::new(n)).unwrap_or(d)
}

/// Date JS « new Date(y, m, d) » tolérante (jour hors mois → débordement sur le mois suivant).
fn ymd(y: i32, m0: u32, d: u32) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(y + (m0 / 12) as i32, m0 % 12 + 1, 1).unwrap_or_else(|| Local::now().date_naive());
    first + Duration::days(d as i64 - 1)
}

/// Jour désigné : « aujourd'hui », « demain », « lundi prochain », « le 25 », « le 3 octobre », « 25/09 ».
pub fn parse_day(text: &str) -> Option<Day> {
    parse_day_at(text, Local::now().date_naive())
}

pub fn parse_day_at(text: &str, base: NaiveDate) -> Option<Day> {
    let t = format!(" {text} ");
    let shift = |n: i64, label: &str| day(base + Duration::days(n), label.to_string());
    if crate::re!(r"\bapres demain\b").is_match(&t) {
        return Some(shift(2, "après-demain"));
    }
    if crate::re!(r"\bavant hier\b").is_match(&t) {
        return Some(shift(-2, "avant-hier"));
    }
    if crate::re!(r"\bdemain\b").is_match(&t) {
        return Some(shift(1, "demain"));
    }
    if crate::re!(r"\bhier\b").is_match(&t) {
        return Some(shift(-1, "hier"));
    }
    if crate::re!(r"\b(?:aujourd hui|ce jour|ce soir|ce matin|cet apres midi)\b").is_match(&t) {
        return Some(shift(0, "aujourd'hui"));
    }
    if let Some(m) = crate::re!(r"\b(\d{1,2})\s*/\s*(\d{1,2})(?:\s*/\s*(\d{2,4}))?\b").captures(&t) {
        let year = m.get(3).map(|y| if y.as_str().len() == 2 { 2000 + y.as_str().parse::<i32>().unwrap_or(0) } else { y.as_str().parse().unwrap_or(base.year()) }).unwrap_or(base.year());
        let mut d = ymd(year, m[2].parse::<u32>().unwrap_or(1).saturating_sub(1), m[1].parse().unwrap_or(1));
        if m.get(3).is_none() && d < base {
            d = ymd(year + 1, d.month0(), d.day());
        }
        return Some(day(d, format_day(d)));
    }
    static DAY_MONTH: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(&format!(r"\b(?:le\s+)?(\d{{1,2}}|premier)(?:er)?\s+({})(?:\s+(\d{{4}}))?\b", MONTHS.join("|"))).unwrap());
    if let Some(m) = DAY_MONTH.captures(&t) {
        let dd = if &m[1] == "premier" { 1 } else { m[1].parse().unwrap_or(1) };
        let mi = MONTHS.iter().position(|x| *x == &m[2]).unwrap_or(0) as u32;
        let y = m.get(3).and_then(|y| y.as_str().parse().ok()).unwrap_or(base.year());
        let mut d = ymd(y, mi, dd);
        if m.get(3).is_none() && d < base {
            d = ymd(y + 1, mi, dd);
        }
        return Some(day(d, format_day(d)));
    }
    static WEEKDAY: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(&format!(r"\b({})(\s+prochain)?\b", DAYS.join("|"))).unwrap());
    if let Some(m) = WEEKDAY.captures(&t) {
        let target = DAYS.iter().position(|x| *x == &m[1]).unwrap_or(0) as i64;
        let mut diff = (target - base.weekday().num_days_from_sunday() as i64 + 7) % 7;
        if diff == 0 && m.get(2).is_some() {
            diff = 7;
        }
        return Some(shift(diff, if diff == 0 { "aujourd'hui" } else { &m[1] }));
    }
    if let Ok(Some(m)) = crate::fre!(r"\ble\s+(\d{1,2})\b(?!\s*(?:h|heures?|:))").captures(&t) {
        let dd: u32 = m[1].parse().unwrap_or(1);
        let mut d = ymd(base.year(), base.month0(), dd);
        if d < base {
            d = add_months(ymd(base.year(), base.month0(), 1), 1) + Duration::days(dd as i64 - 1);
        }
        return Some(day(d, format_day(d)));
    }
    None
}

/// Heure : « 14h », « 14 h 30 », « 9h15 », « 14:30 », « midi » → « HH:MM ».
pub fn parse_time(text: &str) -> Option<String> {
    let t = format!(" {text} ");
    if crate::re!(r"\bmidi\b").is_match(&t) {
        return Some("12:00".into());
    }
    if crate::re!(r"\bminuit\b").is_match(&t) {
        return Some("00:00".into());
    }
    let m = crate::re!(r"\b(\d{1,2})\s*(?:h|heures?|:)\s*(\d{2})?\b").captures(&t)?;
    let mut h: u32 = m[1].parse().ok()?;
    let min: u32 = m.get(2).and_then(|x| x.as_str().parse().ok()).unwrap_or(0);
    if crate::re!(r"\b(?:de l apres midi|du soir)\b").is_match(&t) && h < 12 {
        h += 12;
    }
    if h > 23 || min > 59 {
        return None;
    }
    Some(format!("{h:02}:{min:02}"))
}

// --- Calcul ------------------------------------------------------------------------------------------

fn tokenize_math(expr: &str) -> Option<Vec<String>> {
    let s = format!(" {expr} ");
    let s = crate::re!(r"(\d)\s+(\d{3})\b").replace_all(&s, "$1$2");
    let s = crate::re!(r"(\d),(\d)").replace_all(&s, "$1.$2");
    let s = crate::re!(r"\bracine(?: carree)? de\b").replace_all(&s, " √ ");
    let s = crate::re!(r"\bau carre\b").replace_all(&s, " ^ 2 ");
    let s = crate::re!(r"\bau cube\b").replace_all(&s, " ^ 3 ");
    let s = crate::re!(r"\b(?:puissance|exposant)\b").replace_all(&s, " ^ ");
    let s = crate::re!(r"\b(?:plus)\b").replace_all(&s, " + ");
    let s = crate::re!(r"\b(?:moins)\b").replace_all(&s, " - ");
    let s = crate::re!(r"\b(?:fois|multiplie par|multiplies par|x)\b").replace_all(&s, " * ");
    let s = crate::re!(r"\b(?:divise par|divises par|sur)\b").replace_all(&s, " / ");
    let mut s = s.replace('×', " * ").replace('÷', " / ");
    for w in ["un", "une", "deux", "trois", "quatre", "cinq", "six", "sept", "huit", "neuf", "dix", "onze", "douze", "treize", "quatorze", "quinze", "seize", "vingt", "trente", "quarante", "cinquante", "soixante", "cent", "mille", "premier"] {
        let re = regex::Regex::new(&format!(r"\b{w}\b")).unwrap();
        s = re.replace_all(&s, format!(" {} ", js::num_str(word_number(w).unwrap()))).into_owned();
    }
    let tok = crate::re!(r"\d+(?:\.\d+)?|[-+*/^()%√]");
    let tokens: Vec<String> = tok.find_iter(&s).map(|m| m.as_str().to_string()).collect();
    let leftover = crate::re!(r"\d+(?:\.\d+)?|[-+*/^()%√]|\s+").replace_all(&s, "");
    if !leftover.is_empty() {
        return None;
    }
    Some(tokens)
}

struct P {
    t: Vec<String>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<&str> {
        self.t.get(self.i).map(|s| s.as_str())
    }
    fn next(&mut self) -> Option<String> {
        let r = self.t.get(self.i).cloned();
        self.i += 1;
        r
    }
    fn primary(&mut self) -> Result<f64, ()> {
        let tok = self.next().ok_or(())?;
        match tok.as_str() {
            "(" => {
                let v = self.sum()?;
                if self.next().as_deref() != Some(")") {
                    return Err(());
                }
                Ok(v)
            }
            "-" => Ok(-self.factor()?),
            "+" => self.factor(),
            "√" => Ok(self.factor()?.sqrt()),
            t if t.starts_with(|c: char| c.is_ascii_digit()) => {
                let mut v: f64 = t.parse().map_err(|_| ())?;
                if self.peek() == Some("%") {
                    self.next();
                    v /= 100.0;
                }
                Ok(v)
            }
            _ => Err(()),
        }
    }
    fn factor(&mut self) -> Result<f64, ()> {
        let b = self.primary()?;
        if self.peek() == Some("^") {
            self.next();
            return Ok(b.powf(self.factor()?));
        }
        Ok(b)
    }
    fn product(&mut self) -> Result<f64, ()> {
        let mut v = self.factor()?;
        while matches!(self.peek(), Some("*") | Some("/")) {
            let op = self.next().unwrap();
            let r = self.factor()?;
            v = if op == "*" { v * r } else { v / r };
        }
        Ok(v)
    }
    fn sum(&mut self) -> Result<f64, ()> {
        let mut v = self.product()?;
        while matches!(self.peek(), Some("+") | Some("-")) {
            let op = self.next().unwrap();
            let r = self.product()?;
            v = if op == "+" { v + r } else { v - r };
        }
        Ok(v)
    }
}

/// Analyseur arithmétique maison (jamais d'eval) : + - * / ^ %, parenthèses, racine.
pub fn evaluate_math(expr: &str) -> Option<f64> {
    let tokens = tokenize_math(expr)?;
    if tokens.is_empty() || !tokens.iter().any(|t| t.chars().any(|c| c.is_ascii_digit())) {
        return None;
    }
    let mut p = P { t: tokens, i: 0 };
    let v = p.sum().ok()?;
    if p.i != p.t.len() || !v.is_finite() {
        return None;
    }
    Some(v)
}

/// toLocaleString('fr-FR', { maximumFractionDigits: 6 }).
pub fn format_number(v: f64) -> String {
    let rounded = if v.abs() >= 1e-4 || v == 0.0 { (v * 1e6).round() / 1e6 } else { v };
    let s = format!("{:.6}", rounded.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let frac = frac.trim_end_matches('0');
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push('\u{202f}');
        }
        grouped.push(c);
    }
    let sign = if rounded < 0.0 && (int != "0" || !frac.is_empty()) { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped},{frac}")
    }
}

/// « combien font 12 fois 7 », « calcule 15 % de 240 », « racine carrée de 81 ».
pub fn parse_calculation(text: &str) -> Option<J> {
    let m = crate::re!(r"^(?:calcule|combien (?:font|fait|ca fait)|ca fait combien|c est combien)\s+(.+?)(?:\s+(?:ca fait combien|egal|egale))?$")
        .captures(text)
        .or_else(|| crate::re!(r"^(.+?)\s+(?:egal|egale|ca fait|font)\s+combien$").captures(text))
        .or_else(|| crate::re!(r"^(?:combien (?:font|fait))\s+(.+)$").captures(text));
    let mut expr = m.map(|c| c[1].to_string());
    let src = expr.clone().unwrap_or_else(|| text.to_string());
    if let Some(p) = crate::re!(r"^(?:calcule\s+|combien (?:font|fait)\s+)?(\d+(?:[.,]\d+)?)\s*(?:%|pour ?cent)\s+de\s+(\d+(?:[.,]\d+)?)$").captures(&src) {
        let v = p[1].replace(',', ".").parse::<f64>().unwrap_or(0.0) / 100.0 * p[2].replace(',', ".").parse::<f64>().unwrap_or(0.0);
        return Some(json!({ "expression": format!("{} % de {}", &p[1], &p[2]), "value": v, "text": format_number(v) }));
    }
    if expr.is_none() {
        if crate::re!(r"^\d[\d\s.,]*\s*(?:plus|moins|fois|x|divise par|sur|[-+*/^])\s*\d").is_match(text) {
            expr = Some(text.to_string());
        } else {
            return None;
        }
    }
    let e = crate::re!(r"\bpour ?cent\b").replace_all(&expr.unwrap(), "%").into_owned();
    let v = evaluate_math(&e)?;
    Some(json!({ "expression": e, "value": v, "text": format_number(v) }))
}

// --- Conversions ---------------------------------------------------------------------------------------

fn unit(u: &str) -> Option<(&'static str, f64, &'static str)> {
    Some(match u {
        "km" | "kilometre" | "kilometres" => ("longueur", 1000.0, "kilomètres"),
        "m" | "metre" | "metres" => ("longueur", 1.0, "mètres"),
        "cm" | "centimetre" | "centimetres" => ("longueur", 0.01, "centimètres"),
        "mm" | "millimetres" => ("longueur", 0.001, "millimètres"),
        "mile" | "miles" => ("longueur", 1609.344, "miles"),
        "pied" | "pieds" => ("longueur", 0.3048, "pieds"),
        "pouce" | "pouces" => ("longueur", 0.0254, "pouces"),
        "yard" | "yards" => ("longueur", 0.9144, "yards"),
        "kg" | "kilo" | "kilos" | "kilogramme" | "kilogrammes" => ("masse", 1.0, "kilos"),
        "g" | "gramme" | "grammes" => ("masse", 0.001, "grammes"),
        "livre" | "livres" | "lb" => ("masse", 0.45359237, "livres"),
        "once" | "onces" => ("masse", 0.0283495, "onces"),
        "tonne" | "tonnes" => ("masse", 1000.0, "tonnes"),
        "l" | "litre" | "litres" => ("volume", 1.0, "litres"),
        "cl" | "centilitres" => ("volume", 0.01, "centilitres"),
        "ml" | "millilitres" => ("volume", 0.001, "millilitres"),
        "gallon" | "gallons" => ("volume", 3.785411784, "gallons"),
        "km h" | "kilometres heure" | "kilometres par heure" => ("vitesse", 1.0, "kilomètres heure"),
        "mph" | "miles par heure" | "miles heure" => ("vitesse", 1.609344, "miles par heure"),
        "celsius" | "degres celsius" | "c" => ("temperature", 0.0, "degrés Celsius"),
        "fahrenheit" | "degres fahrenheit" | "f" => ("temperature", 0.0, "degrés Fahrenheit"),
        "kelvin" | "kelvins" => ("temperature", 0.0, "kelvins"),
        _ => return None,
    })
}

const UNIT_KEYS: &[&str] = &[
    "km", "kilometre", "kilometres", "m", "metre", "metres", "cm", "centimetre", "centimetres", "mm", "millimetres", "mile", "miles", "pied", "pieds",
    "pouce", "pouces", "yard", "yards", "kg", "kilo", "kilos", "kilogramme", "kilogrammes", "g", "gramme", "grammes", "livre", "livres", "lb", "once",
    "onces", "tonne", "tonnes", "l", "litre", "litres", "cl", "centilitres", "ml", "millilitres", "gallon", "gallons", "km h", "kilometres heure",
    "kilometres par heure", "mph", "miles par heure", "miles heure", "celsius", "degres celsius", "c", "fahrenheit", "degres fahrenheit", "f",
    "kelvin", "kelvins",
];

static UNIT_PATTERN: LazyLock<String> = LazyLock::new(|| {
    let mut k: Vec<&str> = UNIT_KEYS.to_vec();
    k.sort_by(|a, b| b.len().cmp(&a.len()));
    k.iter().map(|u| u.replace(' ', r"\s")).collect::<Vec<_>>().join("|")
});

fn to_celsius(v: f64, u: &str) -> f64 {
    if u.contains("fahrenheit") || u == "f" {
        (v - 32.0) * 5.0 / 9.0
    } else if u.contains("kelvin") {
        v - 273.15
    } else {
        v
    }
}

fn from_celsius(v: f64, u: &str) -> f64 {
    if u.contains("fahrenheit") || u == "f" {
        v * 9.0 / 5.0 + 32.0
    } else if u.contains("kelvin") {
        v + 273.15
    } else {
        v
    }
}

/// « convertis 10 miles en km », « 30 degrés celsius en fahrenheit », « combien de grammes dans 3 onces ».
pub fn parse_conversion(text: &str) -> Option<J> {
    static A: LazyLock<regex::Regex> = LazyLock::new(|| {
        let u = &*UNIT_PATTERN;
        regex::Regex::new(&format!(r"^(?:convertis|convertir|conversion de|combien (?:font|fait))?\s*(\d+(?:[.,]\d+)?)\s*({u})\s+(?:en|vers)\s+({u})$")).unwrap()
    });
    static B: LazyLock<regex::Regex> = LazyLock::new(|| {
        let u = &*UNIT_PATTERN;
        regex::Regex::new(&format!(r"^combien (?:de |d )?({u})\s+(?:dans|font|fait|pour)\s+(\d+(?:[.,]\d+)?)\s*({u})$")).unwrap()
    });
    let t = crate::re!(r"\bkm\s*/\s*h\b").replace_all(text, "km h");
    let t = crate::re!(r"\bdegres?\s+(celsius|fahrenheit)\b").replace_all(&t, "$1").into_owned();
    let (value, from, to) = if let Some(m) = A.captures(&t) {
        (m[1].to_string(), m[2].to_string(), m[3].to_string())
    } else {
        let m = B.captures(&t)?;
        (m[2].to_string(), m[3].to_string(), m[1].to_string())
    };
    let from = crate::re!(r"\s+").replace_all(&from, " ").into_owned();
    let to = crate::re!(r"\s+").replace_all(&to, " ").into_owned();
    let a = unit(&from)?;
    let b = unit(&to)?;
    if a.0 != b.0 {
        return None;
    }
    let v: f64 = value.replace(',', ".").parse().ok()?;
    let result = if a.0 == "temperature" { from_celsius(to_celsius(v, &from), &to) } else { v * a.1 / b.1 };
    Some(json!({ "value": result, "text": format!("{} {} font {} {}.", format_number(v), a.2, format_number((result * 100.0).round() / 100.0), b.2) }))
}

// --- Sites web ---------------------------------------------------------------------------------------

pub fn site(name: &str) -> Option<&'static str> {
    Some(match name {
        "gmail" | "boite mail" | "mails" => "https://mail.google.com",
        "youtube" => "https://www.youtube.com",
        "google drive" | "drive" => "https://drive.google.com",
        "google docs" | "google doc" => "https://docs.google.com",
        "google sheets" => "https://sheets.google.com",
        "google agenda" | "google calendar" => "https://calendar.google.com",
        "google maps" | "maps" => "https://maps.google.com",
        "google" => "https://www.google.com",
        "netflix" => "https://www.netflix.com",
        "github" => "https://github.com",
        "chatgpt" => "https://chatgpt.com",
        "claude" => "https://claude.ai",
        "linkedin" => "https://www.linkedin.com",
        "facebook" => "https://www.facebook.com",
        "instagram" => "https://www.instagram.com",
        "twitter" => "https://x.com",
        "wikipedia" | "wikipedia_fr" => "https://fr.wikipedia.org",
        "amazon" => "https://www.amazon.fr",
        "leboncoin" | "le bon coin" => "https://www.leboncoin.fr",
        "deezer" => "https://www.deezer.com",
        "twitch" => "https://www.twitch.tv",
        "whatsapp" | "whatsapp web" => "https://web.whatsapp.com",
        "google traduction" | "traducteur" => "https://translate.google.com",
        "disney plus" | "disney+" | "disney" => "https://www.disneyplus.com",
        "prime video" | "amazon prime" | "amazon prime video" => "https://www.primevideo.com",
        "canal plus" | "canal+" | "mycanal" | "my canal" => "https://www.canalplus.com",
        "apple tv" => "https://tv.apple.com",
        "crunchyroll" => "https://www.crunchyroll.com",
        "molotov" => "https://www.molotov.tv",
        "france tv" => "https://www.france.tv",
        "arte" => "https://www.arte.tv",
        "tf1" | "tf1 plus" => "https://www.tf1.fr",
        "m6 plus" => "https://www.6play.fr",
        "allocine" => "https://www.allocine.fr",
        "youtube music" => "https://music.youtube.com",
        "sncf" | "sncf connect" => "https://www.sncf-connect.com",
        "doctolib" => "https://www.doctolib.fr",
        "ameli" => "https://www.ameli.fr",
        "impots" => "https://www.impots.gouv.fr",
        "caf" => "https://www.caf.fr",
        "la poste" => "https://www.laposte.fr",
        "meteo france" => "https://meteofrance.com",
        "vinted" => "https://www.vinted.fr",
        "cdiscount" => "https://www.cdiscount.com",
        "fnac" => "https://www.fnac.com",
        "darty" => "https://www.darty.com",
        "ikea" => "https://www.ikea.com/fr/fr/",
        "booking" => "https://www.booking.com",
        "airbnb" => "https://www.airbnb.fr",
        "google photos" => "https://photos.google.com",
        "outlook" | "hotmail" => "https://outlook.live.com",
        "google keep" => "https://keep.google.com",
        "reddit" => "https://www.reddit.com",
        "tiktok" => "https://www.tiktok.com",
        "pinterest" => "https://www.pinterest.fr",
        "x" => "https://x.com",
        "bluesky" => "https://bsky.app",
        "messenger" => "https://www.messenger.com",
        "telegram" => "https://web.telegram.org",
        "gemini" => "https://gemini.google.com",
        "notion" => "https://www.notion.so",
        "canva" => "https://www.canva.com",
        "figma" => "https://www.figma.com",
        "trello" => "https://trello.com",
        "dropbox" => "https://www.dropbox.com",
        "onedrive" => "https://onedrive.live.com",
        "le monde" => "https://www.lemonde.fr",
        "lequipe" | "l equipe" => "https://www.lequipe.fr",
        _ => return None,
    })
}

fn browser_exe(word: &str) -> Option<&'static str> {
    Some(match word.trim() {
        "chrome" | "google chrome" => "chrome",
        "firefox" | "mozilla firefox" | "mozilla" => "firefox",
        "edge" | "microsoft edge" => "msedge",
        "brave" => "brave",
        "opera" => "opera",
        _ => return None,
    })
}

const BROWSER_WORDS: &str = r"(google chrome|chrome|mozilla firefox|firefox|mozilla|microsoft edge|edge|brave|opera|(?:le |un |mon |ton )?navigateur(?: internet| web)?|internet)";

/// Tournures autour d'un site → « ouvre <site> », et navigateur demandé.
fn web_request(text: &str) -> (String, Option<&'static str>) {
    static A: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(&format!(r"^(?:ouvre|lance|demarre|mets|va sur|va dans)(?: moi)? (?:sur |dans )?{BROWSER_WORDS} (?:sur|avec|a|pour|et va sur|et ouvre|et mets) (.+)$")).unwrap());
    static B: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(&format!(r"^(.+?) (?:dans|sur|avec|via) {BROWSER_WORDS}$")).unwrap());
    let mut t = text.trim().to_string();
    let mut browser = None;
    if let Some(m) = A.captures(&t.clone()) {
        browser = browser_exe(&m[1]);
        t = format!("ouvre {}", &m[2]);
    }
    if let Some(m) = B.captures(&t.clone()) {
        if crate::re!(r"^(?:ouvre|lance|demarre|mets|va sur|affiche|montre)\b").is_match(&m[1]) {
            browser = browser.or_else(|| browser_exe(&m[2]));
            t = m[1].to_string();
        }
    }
    if let Some(m) = crate::re!(r"^(?:lance|mets|regarde|ouvre)(?: moi)? (?:un film|une serie|un episode|une video|un documentaire|un dessin anime|quelque chose|un truc|la tele) (?:sur|dans) (.+)$").captures(&t.clone()) {
        t = format!("ouvre {}", &m[1]);
    }
    if let Some(m) = crate::re!(r"^(?:mets|lance)(?: moi)? (.+)$").captures(&t.clone()) {
        if site(m[1].trim()).is_some() {
            t = format!("ouvre {}", m[1].trim());
        }
    }
    (crate::re!(r"^(ouvre|lance|va sur|affiche|montre)\s+(?:moi\s+)?").replace(&t, "ouvre ").into_owned(), browser)
}

pub fn search_url(engine: &str, q: &str) -> Option<String> {
    let e = urlencoding::encode(q);
    Some(match engine {
        "google" => format!("https://www.google.com/search?q={e}"),
        "youtube" => format!("https://www.youtube.com/results?search_query={e}"),
        "wikipedia" => format!("https://fr.wikipedia.org/w/index.php?search={e}"),
        "amazon" => format!("https://www.amazon.fr/s?k={e}"),
        "google maps" | "maps" => format!("https://www.google.com/maps/search/{e}"),
        "images" => format!("https://www.google.com/search?tbm=isch&q={e}"),
        "leboncoin" => format!("https://www.leboncoin.fr/recherche?text={e}"),
        _ => return None,
    })
}

/// Garde la casse et les accents de la requête d'origine quand on la retrouve dans le texte brut.
fn extract_query(raw: &str, folded: &str) -> String {
    let m = crate::re!(r"(?i)(?:cherche|recherche|trouve|tape|google)\s+(?:moi\s+)?(.+?)\s+(?:sur|dans)\s+\S+(?:\s+maps)?\s*[.!?]*$")
        .captures(raw)
        .or_else(|| crate::re!(r"(?i)(?:recherche google|google)\s+(.+?)\s*[.!?]*$").captures(raw));
    m.map(|c| c[1].to_string()).unwrap_or_else(|| folded.to_string()).trim().to_string()
}

/// « cherche … sur google », « ouvre youtube », « va sur github.com » → { url, label, say, browser? }.
pub fn parse_web(text: &str, raw: &str) -> Option<J> {
    if let Some(m) = crate::re!(r"^(?:cherche|recherche|trouve|tape)\s+(.+?)\s+(?:sur|dans)\s+(google maps|google|youtube|wikipedia|amazon|maps|images|leboncoin)$").captures(text) {
        let engine = m[2].to_string();
        let q = extract_query(raw, &m[1]);
        return Some(json!({ "url": search_url(&engine, &q), "label": format!("Recherche {engine}"), "say": format!("Je cherche {q} sur {}.", if engine == "maps" { "Google Maps".to_string() } else { engine.clone() }), "query": q }));
    }
    if let Some(m) = crate::re!(r"^(?:(?:fais une )?recherche google|google)\s+(.+)$").captures(text) {
        let q = extract_query(raw, &m[1]);
        return Some(json!({ "url": search_url("google", &q), "label": "Recherche Google", "say": format!("Je cherche {q} sur Google."), "query": q }));
    }
    let (req, browser) = web_request(text);
    let via = browser
        .map(|b| {
            let name = ["chrome", "firefox", "edge", "brave", "opera", "mozilla"].iter().find(|k| browser_exe(k) == Some(b)).copied().unwrap_or(b);
            format!(" dans {name}")
        })
        .unwrap_or_default();
    let m = crate::re!(r"^(?:ouvre|lance|va sur|affiche|montre)\s+(?:moi\s+)?(le site\s+(?:de\s+|du\s+|des\s+|d\s+)?|la page\s+(?:de\s+|du\s+|d\s+)?)?(.+?)$").captures(&req)?;
    let spoken = m[2].trim().to_string();
    let name = if site(&spoken).is_some() { spoken.clone() } else { crate::re!(r"^(?:la|le|les|l)\s+").replace(&spoken, "").into_owned() };
    if let Some(url) = site(&name) {
        return Some(json!({ "url": url, "browser": browser, "label": format!("Site {name}"), "say": format!("J'ouvre {name}{via}.") }));
    }
    let domain = crate::re!(r"\s+point\s+").replace_all(&name, ".");
    let domain = crate::re!(r"\s+(com|fr|org|net|io|ai|dev|eu|be|ch|ca|tv|app)$").replace(&domain, ".$1");
    let domain = crate::re!(r"\s+").replace_all(&domain, "").into_owned();
    if crate::re!(r"^[a-z0-9-]+(?:\.[a-z0-9-]+)*\.(?:com|fr|org|net|io|ai|dev|eu|be|ch|ca|tv|app)$").is_match(&domain) {
        return Some(json!({ "url": format!("https://{domain}"), "browser": browser, "label": domain, "say": format!("J'ouvre {domain}{via}.") }));
    }
    if m.get(1).is_some() && name.chars().count() >= 2 {
        let q = crate::re!(r"(?i)(?:site|page)\s+(?:de\s+la\s+|de\s+l['’]\s*|du\s+|des\s+|de\s+|d['’]\s*)?(.+?)(?:\s+(?:dans|sur|avec)\s+.*)?[.!?]*$")
            .captures(raw)
            .map(|c| c[1].to_string())
            .unwrap_or(name.clone())
            .trim()
            .to_string();
        return Some(json!({ "url": format!("https://duckduckgo.com/?q={}", urlencoding::encode(&format!("!ducky {q}"))), "browser": browser, "label": format!("Site {q}"), "say": format!("J'ouvre le site {q}{via}."), "query": q }));
    }
    None
}

// --- Météo ---------------------------------------------------------------------------------------------

pub fn weather_code(c: i64) -> &'static str {
    match c {
        0 => "ciel dégagé",
        1 => "ciel plutôt dégagé",
        2 => "partiellement nuageux",
        3 => "couvert",
        45 => "brouillard",
        48 => "brouillard givrant",
        51 => "bruine légère",
        53 => "bruine",
        55 => "bruine dense",
        56 | 57 => "bruine verglaçante",
        61 => "pluie faible",
        63 => "pluie",
        65 => "forte pluie",
        66 | 67 => "pluie verglaçante",
        71 => "faible neige",
        73 => "neige",
        75 => "forte neige",
        77 => "grains de neige",
        80 | 81 => "averses",
        82 => "fortes averses",
        85 => "averses de neige",
        86 => "fortes averses de neige",
        95 => "orages",
        96 => "orages avec grêle",
        99 => "orages avec forte grêle",
        _ => "",
    }
}

async fn fetch_json(url: &str) -> Result<J, String> {
    let r = crate::net::http().get(url).timeout(std::time::Duration::from_secs(6)).send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("HTTP {}", r.status().as_u16()));
    }
    r.json().await.map_err(|e| e.to_string())
}

pub async fn geocode(city: &str) -> Result<Option<J>, String> {
    let data = fetch_json(&format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=10&language=fr&format=json", urlencoding::encode(city))).await?;
    let results = js::arr(&data["results"]);
    if results.is_empty() {
        return Ok(None);
    }
    let by_pop = |list: Vec<J>| list.into_iter().max_by(|a, b| js::num(&a["population"]).unwrap_or(0.0).partial_cmp(&js::num(&b["population"]).unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    let fr: Vec<J> = results.iter().filter(|r| js::gs(r, "country_code") == "FR").cloned().collect();
    let place = by_pop(fr).or_else(|| by_pop(results)).unwrap();
    Ok(Some(json!({ "name": place["name"], "latitude": place["latitude"], "longitude": place["longitude"], "country": place["country"] })))
}

/// Dernier relevé réussi par ville (lieu, prévisions, heure) : quand Open-Meteo ne répond pas, Jarvis
/// répond d'après ce relevé en disant de quand il date, plutôt que « service indisponible ».
const WEATHER_CACHE_MAX_MS: i64 = 24 * 3_600_000;

struct Reading {
    at_ms: i64,
    place: J,
    data: J,
}

thread_local! {
    static WEATHER_CACHE: std::cell::RefCell<std::collections::HashMap<String, Reading>> = Default::default();
}

fn weather_key(city: &str) -> String {
    js::fold_lower(city.trim())
}

/// « de 8 h 10 », « d'hier à 18 h 05 », « du 19 octobre à 8 h 10 » : de quand date un relevé.
pub fn reading_label(at_ms: i64) -> String {
    const MOIS: [&str; 12] = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];
    let Some(at) = chrono::DateTime::from_timestamp_millis(at_ms).map(|d| d.with_timezone(&Local)) else { return String::new() };
    let hour = format!("{} h {:02}", at.format("%-H"), at.format("%M"));
    let today = Local::now().date_naive();
    match (today - at.date_naive()).num_days() {
        0 => format!("de {hour}"),
        1 => format!("d'hier à {hour}"),
        _ => format!("du {} {} à {hour}", if at.day() == 1 { "1er".to_string() } else { at.day().to_string() }, MOIS[at.month0() as usize]),
    }
}

/// Lieu et prévisions sur 7 jours ; Ok(None) si la ville est inconnue.
async fn fetch_forecast(city: &str) -> Result<Option<(J, J)>, String> {
    let Some(place) = geocode(city).await? else { return Ok(None) };
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,weather_code,wind_speed_10m,precipitation&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max&timezone=auto&forecast_days=7",
        js::s(&place["latitude"]),
        js::s(&place["longitude"])
    );
    let data = fetch_json(&url).await?;
    Ok(Some((place, data)))
}

/// Réponse d'après le dernier relevé gardé (moins de 24 h), sans réseau.
pub fn weather_from_cache(city: &str, day: Option<&Day>) -> Option<J> {
    let (at_ms, place, data) = WEATHER_CACHE.with(|c| c.borrow().get(&weather_key(city)).filter(|r| js::now_ms() - r.at_ms < WEATHER_CACHE_MAX_MS).map(|r| (r.at_ms, r.place.clone(), r.data.clone())))?;
    let mut r = describe_weather(&place, &data, day);
    let say = format!("Le service météo ne répond pas ; d'après le relevé {} : {}", reading_label(at_ms), js::gs(&r, "say"));
    r["say"] = json!(say);
    r["display"] = json!(format!("{} · relevé {}", js::gs(&r, "display"), reading_label(at_ms)));
    r["stale"] = json!(true);
    r["releve"] = json!(js::iso_of(at_ms));
    Some(r)
}

/// day : None pour « maintenant ». Panne réseau : dernier relevé daté s'il en reste un.
pub async fn weather(city: &str, day: Option<&Day>) -> Result<J, String> {
    match fetch_forecast(city).await {
        Ok(None) => Ok(json!({ "ok": false, "say": format!("Je ne trouve pas la ville {city}.") })),
        Ok(Some((place, data))) => {
            let r = describe_weather(&place, &data, day);
            WEATHER_CACHE.with(|c| c.borrow_mut().insert(weather_key(city), Reading { at_ms: js::now_ms(), place, data }));
            Ok(r)
        }
        Err(e) => weather_from_cache(city, day).ok_or(e),
    }
}

fn describe_weather(place: &J, data: &J, day: Option<&Day>) -> J {
    let round = |v: &J| js::num(v).unwrap_or(f64::NAN).round();
    let name = js::gs(place, "name");
    let d = &data["daily"];
    let today = day.map(|x| x.label == "aujourd'hui").unwrap_or(true);
    if today {
        let c = &data["current"];
        let code = weather_code(c["weather_code"].as_i64().unwrap_or(-1));
        let rain = d["precipitation_probability_max"].get(0).cloned().filter(|v| !v.is_null());
        let mut say = format!("À {name}, {}, {} degrés.", if code.is_empty() { "temps incertain" } else { code }, js::num_str(round(&c["temperature_2m"])));
        if js::truthy(&d["temperature_2m_max"]) {
            say.push_str(&format!(" Entre {} et {} aujourd'hui", js::num_str(round(&d["temperature_2m_min"][0])), js::num_str(round(&d["temperature_2m_max"][0]))));
        }
        say.push_str(&match &rain {
            Some(r) => format!(", {} % de risque de pluie.", js::s(r)),
            None => ".".into(),
        });
        return json!({ "ok": true, "say": say, "display": format!("{name} · {}° · {code}", js::num_str(round(&c["temperature_2m"]))), "place": place });
    }
    let day = day.unwrap();
    let Some(idx) = js::arr(&d["time"]).iter().position(|t| js::s(t) == day.iso) else {
        return json!({ "ok": false, "say": "Je n’ai pas de prévision aussi lointaine, sept jours au maximum." });
    };
    let code = weather_code(d["weather_code"][idx].as_i64().unwrap_or(-1));
    let rain = d["precipitation_probability_max"].get(idx).cloned().filter(|v| !v.is_null());
    let say = format!(
        "{} à {name} : {}, entre {} et {} degrés{}",
        js::capitalize(&day.label),
        if code.is_empty() { "temps incertain" } else { code },
        js::num_str(round(&d["temperature_2m_min"][idx])),
        js::num_str(round(&d["temperature_2m_max"][idx])),
        match &rain {
            Some(r) => format!(", {} % de risque de pluie.", js::s(r)),
            None => ".".into(),
        }
    );
    json!({ "ok": true, "say": say, "display": format!("{name} · {} · {}°", day.label, js::num_str(round(&d["temperature_2m_max"][idx]))), "place": place })
}

#[allow(dead_code)]
fn _v(_: Value) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_releve_dit_de_quand_il_date() {
        let now = js::now_ms();
        assert!(reading_label(now).starts_with("de "));
        assert!(reading_label(now - 24 * 3_600_000).starts_with("d'hier à "));
        assert!(reading_label(now - 3 * 24 * 3_600_000).starts_with("du "));
    }

    #[test]
    fn sans_reseau_la_meteo_vient_du_dernier_releve_date() {
        let place = json!({ "name": "Lyon", "latitude": 45.75, "longitude": 4.85 });
        let data = json!({
            "current": { "temperature_2m": 12.4, "weather_code": 3 },
            "daily": { "time": [Local::now().format("%Y-%m-%d").to_string()], "weather_code": [3], "temperature_2m_min": [8.0], "temperature_2m_max": [15.0], "precipitation_probability_max": [20] }
        });
        assert!(weather_from_cache("Lyon", None).is_none(), "rien en cache");
        WEATHER_CACHE.with(|c| c.borrow_mut().insert(weather_key("Lyon"), Reading { at_ms: js::now_ms() - 3_600_000, place: place.clone(), data: data.clone() }));
        let r = weather_from_cache("lyon", None).expect("relevé gardé");
        assert!(js::gb(&r, "stale"));
        // « du relevé de 9 h 10 », ou « d'hier à 23 h 10 » quand le test tourne entre minuit et 1 h.
        let say = js::gs(&r, "say");
        assert!((say.contains("d'après le relevé de ") || say.contains("d'après le relevé d'hier")) && say.contains("12 degrés"), "{say}");
        // Trop ancien : on ne le ressert plus.
        WEATHER_CACHE.with(|c| c.borrow_mut().insert(weather_key("Lyon"), Reading { at_ms: js::now_ms() - 25 * 3_600_000, place, data }));
        assert!(weather_from_cache("Lyon", None).is_none());
    }
}
